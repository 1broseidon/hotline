package main

import (
	"context"
	"testing"
)

// captureSink records the (content, meta) of the last SendChannel, standing in
// for the transport's Notifier so the pi sink can be exercised without stdio.
type captureSink struct {
	content string
	meta    map[string]string
	called  bool
}

func (c *captureSink) SendChannel(ctx context.Context, content string, meta map[string]string) error {
	c.content = content
	c.meta = meta
	c.called = true
	return nil
}

func (c *captureSink) SendVerdict(ctx context.Context, requestID, behavior string) error {
	return nil
}

// TestPiSinkRendersChannelEnvelope is the T2 golden: a synthetic inbound turn
// pushed through the pi sink produces a claude/channel frame whose content is
// the exact <channel …> envelope (routing keys in attribute order, content
// wrapped), with meta dropped — everything the extension needs now rides inside
// the text it forwards to pi.sendUserMessage.
func TestPiSinkRendersChannelEnvelope(t *testing.T) {
	inner := &captureSink{}
	sink := &piSink{next: inner}

	meta := map[string]string{
		"source":     "telegram",
		"chat_id":    "412587349",
		"message_id": "57",
		"user":       "george",
	}
	if err := sink.SendChannel(context.Background(), "hey there", meta); err != nil {
		t.Fatalf("SendChannel: %v", err)
	}
	if !inner.called {
		t.Fatal("pi sink did not forward to the underlying notifier")
	}

	const want = `<channel source="telegram" chat_id="412587349" message_id="57" user="george">
hey there
</channel>`
	if inner.content != want {
		t.Errorf("channel frame content mismatch:\n got %q\nwant %q", inner.content, want)
	}
	if inner.meta != nil {
		t.Errorf("meta must be dropped after rendering (rides inside the envelope); got %v", inner.meta)
	}
}

// TestPiSinkRendersScheduleFire proves schedule/notify fires ride the same
// envelope path: a kind="schedule" turn renders its kind into the tag.
func TestPiSinkRendersScheduleFire(t *testing.T) {
	inner := &captureSink{}
	sink := &piSink{next: inner}

	meta := map[string]string{"source": "telegram", "chat_id": "1", "kind": "schedule"}
	if err := sink.SendChannel(context.Background(), "time to check in", meta); err != nil {
		t.Fatalf("SendChannel: %v", err)
	}
	const want = `<channel source="telegram" chat_id="1" kind="schedule">
time to check in
</channel>`
	if inner.content != want {
		t.Errorf("schedule fire envelope mismatch:\n got %q\nwant %q", inner.content, want)
	}
}

// TestHarnessBindsSDKApply (pi hot-apply amendment 2026-07-20): the forwarder
// binding is the hot path's on-switch. It must cover exactly the harnesses that
// implement the sdk_apply control — claude-sdk (live Agent SDK Query) and pi
// (ExtensionAPI setModel/setThinkingLevel). Binding for a harness that cannot
// answer would strand every request on the 10 s pending timeout; NOT binding for
// one that can would drop it onto the restart path the "no restart for a
// model/effort change" directive bans.
func TestHarnessBindsSDKApply(t *testing.T) {
	for _, label := range []string{"claude-sdk", "pi"} {
		if !harnessBindsSDKApply(label) {
			t.Errorf("harness %q implements sdk_apply but does not bind the forwarder", label)
		}
	}
	for _, label := range []string{"opencode", "claude", "", "future-thing"} {
		if harnessBindsSDKApply(label) {
			t.Errorf("harness %q does not implement sdk_apply; binding would strand requests on the pending timeout", label)
		}
	}
}
