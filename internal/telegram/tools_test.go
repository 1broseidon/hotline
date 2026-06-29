package telegram

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/PaulSonOfLars/gotgbot/v2"

	"example.com/tele-go/internal/access"
	"example.com/tele-go/internal/config"
	"example.com/tele-go/internal/mcpchan"
)

func newTestCfg(t *testing.T) *config.Config {
	t.Helper()
	dir := t.TempDir()
	inbox := filepath.Join(dir, "inbox")
	if err := os.MkdirAll(inbox, 0o700); err != nil {
		t.Fatal(err)
	}
	return &config.Config{
		StateDir:   dir,
		AccessFile: filepath.Join(dir, "access.json"),
		InboxDir:   inbox,
	}
}

func writeAccess(t *testing.T, cfg *config.Config, a *access.Access) {
	t.Helper()
	if err := access.Save(a, cfg.AccessFile); err != nil {
		t.Fatal(err)
	}
}

func TestToolsNoTokenBranches(t *testing.T) {
	cfg := newTestCfg(t)
	tools := NewTools(nil, cfg, nil) // nil bot == no token
	ctx := context.Background()

	if msg, isErr := tools.Reply(ctx, mcpchan.ReplyInput{ChatID: "1", Text: "x"}); !isErr || !strings.Contains(msg, "no bot token configured") {
		t.Errorf("Reply no-token: %q isErr=%v", msg, isErr)
	}
	if msg, isErr := tools.React(ctx, mcpchan.ReactInput{ChatID: "1", MessageID: "2", Emoji: "👍"}); !isErr || !strings.Contains(msg, "no bot token configured") {
		t.Errorf("React no-token: %q isErr=%v", msg, isErr)
	}
	if msg, isErr := tools.EditMessage(ctx, mcpchan.EditInput{ChatID: "1", MessageID: "2", Text: "x"}); !isErr || !strings.Contains(msg, "no bot token configured") {
		t.Errorf("EditMessage no-token: %q isErr=%v", msg, isErr)
	}
	if msg, isErr := tools.DownloadAttachment(ctx, mcpchan.DownloadInput{FileID: "f"}); !isErr || !strings.Contains(msg, "no bot token configured") {
		t.Errorf("DownloadAttachment no-token: %q isErr=%v", msg, isErr)
	}
}

func TestToolsInvalidIDs(t *testing.T) {
	cfg := newTestCfg(t)
	tools := NewTools(&gotgbot.Bot{}, cfg, nil) // non-nil bot, no network reached
	ctx := context.Background()

	if msg, isErr := tools.Reply(ctx, mcpchan.ReplyInput{ChatID: "notnum", Text: "x"}); !isErr || !strings.Contains(msg, "invalid chat_id") {
		t.Errorf("Reply bad chat_id: %q isErr=%v", msg, isErr)
	}
	if msg, isErr := tools.React(ctx, mcpchan.ReactInput{ChatID: "1", MessageID: "bad", Emoji: "👍"}); !isErr || !strings.Contains(msg, "invalid message_id") {
		t.Errorf("React bad message_id: %q isErr=%v", msg, isErr)
	}
	if msg, isErr := tools.EditMessage(ctx, mcpchan.EditInput{ChatID: "bad", MessageID: "2", Text: "x"}); !isErr || !strings.Contains(msg, "invalid chat_id") {
		t.Errorf("EditMessage bad chat_id: %q isErr=%v", msg, isErr)
	}
}

// TestReplyRejectsUnallowlistedChat confirms the outbound gate fires (and no
// network call is attempted) when the target chat isn't allowlisted.
func TestReplyRejectsUnallowlistedChat(t *testing.T) {
	cfg := newTestCfg(t)
	writeAccess(t, cfg, &access.Access{DMPolicy: "pairing", AllowFrom: []string{"999"}})
	tools := NewTools(&gotgbot.Bot{}, cfg, nil)

	msg, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{ChatID: "1", Text: "x"})
	if !isErr || !strings.Contains(msg, "not allowlisted") {
		t.Errorf("expected allowlist rejection, got %q isErr=%v", msg, isErr)
	}
}

func TestAssertAllowedChat(t *testing.T) {
	cfg := newTestCfg(t)
	writeAccess(t, cfg, &access.Access{
		DMPolicy:  "pairing",
		AllowFrom: []string{"42"},
		Groups:    map[string]access.GroupPolicy{"-100": {}},
	})
	tools := NewTools(nil, cfg, nil)

	if err := tools.assertAllowedChat("42"); err != nil {
		t.Errorf("allowlisted DM should pass: %v", err)
	}
	if err := tools.assertAllowedChat("-100"); err != nil {
		t.Errorf("configured group should pass: %v", err)
	}
	if err := tools.assertAllowedChat("7"); err == nil {
		t.Error("unknown chat should be rejected")
	}
}

func TestAssertSendable(t *testing.T) {
	cfg := newTestCfg(t)
	tools := NewTools(nil, cfg, nil)

	// A state file outside inbox is refused.
	stateFile := filepath.Join(cfg.StateDir, "access.json")
	if err := os.WriteFile(stateFile, []byte("{}"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := tools.assertSendable(stateFile); err == nil {
		t.Error("sending a state file should be refused")
	}

	// A file inside the inbox is allowed.
	inboxFile := filepath.Join(cfg.InboxDir, "photo.png")
	if err := os.WriteFile(inboxFile, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := tools.assertSendable(inboxFile); err != nil {
		t.Errorf("inbox file should be sendable: %v", err)
	}

	// A nonexistent file is left for os.Stat to report later (no error here).
	if err := tools.assertSendable(filepath.Join(cfg.StateDir, "nope.dat")); err != nil {
		t.Errorf("nonexistent path should not trip assertSendable: %v", err)
	}

	// A file entirely outside the state dir is fine.
	outside := filepath.Join(t.TempDir(), "external.txt")
	if err := os.WriteFile(outside, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := tools.assertSendable(outside); err != nil {
		t.Errorf("external file should be sendable: %v", err)
	}
}

func TestReplyRejectsOversizeFile(t *testing.T) {
	cfg := newTestCfg(t)
	writeAccess(t, cfg, &access.Access{DMPolicy: "pairing", AllowFrom: []string{"1"}})
	tools := NewTools(&gotgbot.Bot{}, cfg, nil)

	big := filepath.Join(cfg.InboxDir, "big.bin")
	f, err := os.Create(big)
	if err != nil {
		t.Fatal(err)
	}
	// Sparse file just over 50MB so we don't actually allocate it.
	if err := f.Truncate(maxAttachmentBytes + 1); err != nil {
		t.Fatal(err)
	}
	f.Close()

	msg, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{ChatID: "1", Text: "x", Files: []string{big}})
	if !isErr || !strings.Contains(msg, "too large") {
		t.Errorf("expected oversize rejection, got %q isErr=%v", msg, isErr)
	}
}

func TestJoinInts(t *testing.T) {
	if got := joinInts([]int64{1, 2, 3}); got != "1, 2, 3" {
		t.Fatalf("joinInts = %q", got)
	}
	if got := joinInts(nil); got != "" {
		t.Fatalf("joinInts(nil) = %q", got)
	}
}
