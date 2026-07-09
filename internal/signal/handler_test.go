package signal

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"strings"
	"testing"

	"github.com/1broseidon/hotline/internal/access"
	"github.com/1broseidon/hotline/internal/mcpchan"
)

func TestHandleEnvelopeDMNormalization(t *testing.T) {
	h, _, _, _ := testEnv(t, func(a *access.Access) {
		a.DMPolicy = "allowlist"
		a.AllowFrom = []string{"+15550002222"}
	})
	bursts := captureBursts(h)

	e := dmEnvelope("+15550002222", "George", "hello there.", 1782986400000)
	e.DataMessage.Quote = &quote{ID: 1782986300000, AuthorNumber: testAccount, Text: "earlier text"}
	h.HandleEnvelope(context.Background(), e)
	h.FlushAll(context.Background()) // complete message now takes the grace hold; drain it

	if len(*bursts) != 1 || len((*bursts)[0]) != 1 {
		t.Fatalf("bursts %v", *bursts)
	}
	got := (*bursts)[0][0]
	if got.content != "hello there." {
		t.Fatalf("content %q", got.content)
	}
	for k, want := range map[string]string{
		"chat_id":             "+15550002222",
		"message_id":          "1782986400000:+15550002222",
		"user":                "George",
		"user_id":             "+15550002222",
		"ts":                  "2026-07-02T10:00:00.000Z",
		"reply_to_message_id": "1782986300000:" + testAccount,
		"reply_to_from":       "you",
		"reply_to_text":       "earlier text",
	} {
		if got.meta[k] != want {
			t.Errorf("meta[%s] = %q, want %q", k, got.meta[k], want)
		}
	}
}

func TestHandleEnvelopeGroupNormalization(t *testing.T) {
	h, _, _, _ := testEnv(t, func(a *access.Access) {
		a.Groups = map[string]access.GroupPolicy{"group:R2x2==": {}}
	})
	bursts := captureBursts(h)

	h.HandleEnvelope(context.Background(), groupEnvelope("+15550003333", "R2x2==", "group hi.", 1782986400000))
	h.FlushAll(context.Background()) // complete message now takes the grace hold; drain it

	if len(*bursts) != 1 {
		t.Fatalf("bursts %v", *bursts)
	}
	meta := (*bursts)[0][0].meta
	if meta["chat_id"] != "group:R2x2==" {
		t.Fatalf("chat_id %q", meta["chat_id"])
	}
	if meta["user_id"] != "+15550003333" {
		t.Fatalf("user_id %q", meta["user_id"])
	}
}

func TestHandleEnvelopeDropsUnknownSenderAndPairs(t *testing.T) {
	h, _, d, _ := testEnv(t, func(a *access.Access) {
		a.DMPolicy = "pairing"
		a.AllowFrom = []string{"+15550002222"}
	})
	bursts := captureBursts(h)

	h.HandleEnvelope(context.Background(), dmEnvelope("+15550007777", "", "hi.", 1))
	if len(*bursts) != 0 {
		t.Fatal("stranger relayed")
	}
	sends := d.callsFor("send")
	if len(sends) != 1 || !strings.Contains(sends[0].Params["message"].(string), "--provider signal") {
		t.Fatalf("pairing reply %v", sends)
	}

	acc, err := access.Load(h.Cfg.AccessFile)
	if err != nil {
		t.Fatal(err)
	}
	if len(acc.Pending) != 1 {
		t.Fatalf("pending %v", acc.Pending)
	}
}

func TestHandleEnvelopeIgnoresOwnAndEmpty(t *testing.T) {
	h, _, _, _ := testEnv(t, func(a *access.Access) { a.AllowFrom = []string{testAccount} })
	bursts := captureBursts(h)
	h.HandleEnvelope(context.Background(), dmEnvelope(testAccount, "", "sync echo.", 1))
	h.HandleEnvelope(context.Background(), &envelope{SourceNumber: "+15550002222"}) // no dataMessage
	if len(*bursts) != 0 {
		t.Fatalf("relayed %v", *bursts)
	}
}

func TestPermissionTextReply(t *testing.T) {
	h, _, d, sink := testEnv(t, func(a *access.Access) {
		a.AllowFrom = []string{"+15550002222"}
	})

	h.OnPermissionRequest(context.Background(), mcpchan.PermissionRequestParams{
		RequestID: "abcde", ToolName: "Bash", Description: "run ls", InputPreview: "ls",
	})
	prompts := d.callsFor("send")
	if len(prompts) != 1 {
		t.Fatalf("prompts %v", prompts)
	}
	body := prompts[0].Params["message"].(string)
	if !strings.Contains(body, "yes abcde") || !strings.Contains(body, "no abcde") {
		t.Fatalf("prompt lacks text-answer instructions: %q", body)
	}

	h.HandleEnvelope(context.Background(), dmEnvelope("+15550002222", "", "yes abcde", 5))
	if len(sink.Verdicts) != 1 || sink.Verdicts[0] != [2]string{"abcde", "allow"} {
		t.Fatalf("verdicts %v", sink.Verdicts)
	}
	// Confirmation reaction went out.
	if len(d.callsFor("sendReaction")) != 1 {
		t.Fatal("no confirmation reaction")
	}

	// Second answer finds the claim gone -> shrug, no second verdict.
	h.HandleEnvelope(context.Background(), dmEnvelope("+15550002222", "", "no abcde", 6))
	if len(sink.Verdicts) != 1 {
		t.Fatalf("verdicts %v", sink.Verdicts)
	}
}

func TestPermissionReplyFromStrangerIgnored(t *testing.T) {
	h, _, _, sink := testEnv(t, func(a *access.Access) {
		a.DMPolicy = "allowlist"
		a.AllowFrom = []string{"+15550002222"}
	})
	h.OnPermissionRequest(context.Background(), mcpchan.PermissionRequestParams{RequestID: "abcde", ToolName: "Bash"})
	h.HandleEnvelope(context.Background(), dmEnvelope("+15550007777", "", "yes abcde", 5))
	if len(sink.Verdicts) != 0 {
		t.Fatalf("stranger answered a permission: %v", sink.Verdicts)
	}
}

// TestNumberedOptionsRoundTrip drives the full degradation loop: buttons in a
// reply become numbered text, and the user's bare-number answer comes back to
// Claude as the chosen label.
func TestNumberedOptionsRoundTrip(t *testing.T) {
	h, tools, d, sink := testEnv(t, func(a *access.Access) {
		a.AllowFrom = []string{"+15550002222"}
		a.BubbleMode = "instant"
	})

	out, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{
		ChatID:  "+15550002222",
		Bubbles: []string{"ship it?"},
		Buttons: []string{"ship it", "not yet"},
	})
	if isErr {
		t.Fatalf("reply errored: %s", out)
	}
	sent := d.callsFor("send")[0].Params["message"].(string)
	if !strings.Contains(sent, "1. ship it") || !strings.Contains(sent, "2. not yet") {
		t.Fatalf("options not rendered as numbered text: %q", sent)
	}

	h.HandleEnvelope(context.Background(), dmEnvelope("+15550002222", "George", "2", 9))
	if len(sink.Contents) != 1 || sink.Contents[0] != "not yet" {
		t.Fatalf("answer relay %v", sink.Contents)
	}
	if sink.Metas[0]["kind"] != "button" {
		t.Fatalf("kind %q", sink.Metas[0]["kind"])
	}

	// Options are consumed: a later "2" is an ordinary message again.
	bursts := captureBursts(h)
	h.HandleEnvelope(context.Background(), dmEnvelope("+15550002222", "George", "2", 10))
	h.FlushAll(context.Background())
	if len(*bursts) != 1 || (*bursts)[0][0].content != "2" {
		t.Fatalf("consumed options replayed: %v", *bursts)
	}
}

func TestNumberedOptionsOutOfRangePassesThrough(t *testing.T) {
	h, tools, _, _ := testEnv(t, func(a *access.Access) {
		a.AllowFrom = []string{"+15550002222"}
		a.BubbleMode = "instant"
	})
	tools.Options.set("+15550002222", []string{"a", "b"})
	bursts := captureBursts(h)
	h.HandleEnvelope(context.Background(), dmEnvelope("+15550002222", "", "7", 9))
	h.FlushAll(context.Background())
	if len(*bursts) != 1 || (*bursts)[0][0].content != "7" {
		t.Fatalf("out-of-range not passed through: %v", *bursts)
	}
	_ = tools
}

func TestHandleEnvelopeAttachmentMeta(t *testing.T) {
	h, _, d, _ := testEnv(t, func(a *access.Access) {
		a.AllowFrom = []string{"+15550002222"}
	})
	bursts := captureBursts(h)

	// Non-image: lazy, surfaced by packed file_id.
	e := dmEnvelope("+15550002222", "", "", 11)
	e.DataMessage.Attachments = []attachment{{ContentType: "application/pdf", Filename: "notes.pdf", ID: "att9", Size: 1234}}
	h.HandleEnvelope(context.Background(), e)
	h.FlushAll(context.Background())
	got := (*bursts)[0][0]
	if got.content != "(document: notes.pdf)" {
		t.Fatalf("content %q", got.content)
	}
	if got.meta["attachment_file_id"] != "att9|+15550002222" {
		t.Fatalf("file_id %q", got.meta["attachment_file_id"])
	}
	if got.meta["attachment_kind"] != "document" || got.meta["attachment_mime"] != "application/pdf" {
		t.Fatalf("meta %v", got.meta)
	}

	// Image: eagerly fetched through getAttachment into the inbox.
	d.Results["getAttachment"] = base64.StdEncoding.EncodeToString([]byte("IMG"))
	e2 := dmEnvelope("+15550002222", "", "look", 12)
	e2.DataMessage.Attachments = []attachment{{ContentType: "image/png", Filename: "pic.png", ID: "att10"}}
	h.HandleEnvelope(context.Background(), e2)
	h.FlushAll(context.Background())
	got2 := (*bursts)[1][0]
	if got2.meta["image_path"] == "" || !strings.Contains(got2.meta["image_path"], h.Cfg.InboxDir) {
		t.Fatalf("image_path %q", got2.meta["image_path"])
	}
}

func TestHandleEventFiltersOtherAccounts(t *testing.T) {
	h, _, _, _ := testEnv(t, func(a *access.Access) { a.AllowFrom = []string{"+15550002222"} })
	bursts := captureBursts(h)

	mk := func(account string) sseEvent {
		payload, _ := json.Marshal(receivePayload{
			Envelope: *dmEnvelope("+15550002222", "", "hi.", 1),
			Account:  account,
		})
		return sseEvent{Event: "receive", Data: string(payload)}
	}
	h.HandleEvent(context.Background(), mk("+19998887777"))
	if len(*bursts) != 0 {
		t.Fatal("foreign account relayed")
	}
	h.HandleEvent(context.Background(), mk(testAccount))
	h.FlushAll(context.Background()) // complete message now takes the grace hold; drain it
	if len(*bursts) != 1 {
		t.Fatal("own account not relayed")
	}
}
