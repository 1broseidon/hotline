package signal

import (
	"context"
	"testing"

	"github.com/1broseidon/hotline/internal/access"
)

// reactionEnvelope builds a reaction envelope the way signal-cli delivers one:
// a dataMessage with no message text and a populated reaction record.
func reactionEnvelope(sender, name, emoji, targetAuthor string, targetTS, ts int64, isRemove bool) *envelope {
	return &envelope{
		SourceNumber: sender,
		SourceName:   name,
		Timestamp:    ts,
		DataMessage: &dataMessage{
			Timestamp: ts,
			Reaction: &reaction{
				Emoji:               emoji,
				TargetAuthor:        targetAuthor,
				TargetAuthorNumber:  targetAuthor,
				TargetSentTimestamp: targetTS,
				IsRemove:            isRemove,
			},
		},
	}
}

func TestHandleEnvelopeReactionAdded(t *testing.T) {
	h, _, _, sink := testEnv(t, func(a *access.Access) {
		a.DMPolicy = "allowlist"
		a.AllowFrom = []string{"+15550002222"}
	})

	// Reacting to one of the bot's own messages: target id is the bare
	// timestamp, matching the id shape outbound sends return.
	h.HandleEnvelope(context.Background(),
		reactionEnvelope("+15550002222", "George", "👍", testAccount, 1782986300000, 1782986400000, false))

	if len(sink.Contents) != 1 {
		t.Fatalf("contents %v", sink.Contents)
	}
	if sink.Contents[0] != "👍" {
		t.Fatalf("content %q", sink.Contents[0])
	}
	for k, want := range map[string]string{
		"chat_id":           "+15550002222",
		"user":              "George",
		"user_id":           "+15550002222",
		"ts":                "2026-07-02T10:00:00.000Z",
		"kind":              "reaction",
		"reaction":          "added",
		"target_message_id": "1782986300000",
	} {
		if sink.Metas[0][k] != want {
			t.Errorf("meta[%s] = %q, want %q", k, sink.Metas[0][k], want)
		}
	}
	if _, ok := sink.Metas[0]["message_id"]; ok {
		t.Error("reaction carries message_id; it must only carry target_message_id")
	}
}

func TestHandleEnvelopeReactionRemoved(t *testing.T) {
	h, _, _, sink := testEnv(t, func(a *access.Access) {
		a.AllowFrom = []string{"+15550002222"}
	})

	h.HandleEnvelope(context.Background(),
		reactionEnvelope("+15550002222", "George", "🔥", testAccount, 1782986300000, 1782986400000, true))

	if len(sink.Contents) != 1 || sink.Contents[0] != "🔥" {
		t.Fatalf("contents %v", sink.Contents)
	}
	if sink.Metas[0]["reaction"] != "removed" {
		t.Fatalf("reaction %q", sink.Metas[0]["reaction"])
	}
}

func TestHandleEnvelopeReactionToOtherAuthor(t *testing.T) {
	h, _, _, sink := testEnv(t, func(a *access.Access) {
		a.Groups = map[string]access.GroupPolicy{"group:R2x2==": {}}
	})

	// A group member reacting to another member's message: target id uses the
	// inbound "<timestamp>:<author>" composite.
	e := reactionEnvelope("+15550003333", "", "😂", "+15550004444", 1782986300000, 1782986400000, false)
	e.DataMessage.GroupInfo = &groupInfo{GroupID: "R2x2==", Type: "DELIVER"}
	h.HandleEnvelope(context.Background(), e)

	if len(sink.Metas) != 1 {
		t.Fatalf("metas %v", sink.Metas)
	}
	if got := sink.Metas[0]["target_message_id"]; got != "1782986300000:+15550004444" {
		t.Fatalf("target_message_id %q", got)
	}
	if sink.Metas[0]["chat_id"] != "group:R2x2==" {
		t.Fatalf("chat_id %q", sink.Metas[0]["chat_id"])
	}
}

func TestHandleEnvelopeReactionFromStrangerDropped(t *testing.T) {
	h, _, d, sink := testEnv(t, func(a *access.Access) {
		a.DMPolicy = "pairing"
		a.AllowFrom = []string{"+15550002222"}
	})

	h.HandleEnvelope(context.Background(),
		reactionEnvelope("+15550007777", "", "👍", testAccount, 100, 200, false))

	if len(sink.Contents) != 0 {
		t.Fatalf("stranger reaction relayed: %v", sink.Contents)
	}
	// A reaction never starts a pairing, even under the pairing DM policy.
	if sends := d.callsFor("send"); len(sends) != 0 {
		t.Fatalf("pairing reply sent for a reaction: %v", sends)
	}
	acc, err := access.Load(h.Cfg.AccessFile)
	if err != nil {
		t.Fatal(err)
	}
	if len(acc.Pending) != 0 {
		t.Fatalf("pending pairing created by a reaction: %v", acc.Pending)
	}
}

func TestHandleEnvelopeReactionUnconfiguredGroupDropped(t *testing.T) {
	h, _, _, sink := testEnv(t, func(a *access.Access) {
		a.AllowFrom = []string{"+15550002222"}
	})

	e := reactionEnvelope("+15550002222", "", "👍", testAccount, 100, 200, false)
	e.DataMessage.GroupInfo = &groupInfo{GroupID: "NOPE==", Type: "DELIVER"}
	h.HandleEnvelope(context.Background(), e)

	if len(sink.Contents) != 0 {
		t.Fatalf("unconfigured-group reaction relayed: %v", sink.Contents)
	}
}
