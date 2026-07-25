package app

import (
	"context"
	"encoding/json"
	"testing"
)

// sentPayload pulls the sent-echo fields out of an enqueued mailbox item.
func sentPayload(t *testing.T, item MailboxItem) (typ, device, text, cid string) {
	t.Helper()
	var p struct {
		T      string `json:"t"`
		Device string `json:"device"`
		Text   string `json:"text"`
		CID    string `json:"cid"`
	}
	if err := json.Unmarshal(item.Payload, &p); err != nil {
		t.Fatalf("decode sent payload: %v", err)
	}
	return p.T, p.Device, p.Text, p.CID
}

// TestOwnSendMirrorToSiblings proves the box-side own-send mirror the design
// claims (§3.2): a device_send from device A journals a `sent` echo carrying
// origin device=A that lands in EVERY active device's mailbox — the sender and
// its online siblings alike — so all screens render the user's own message.
func TestOwnSendMirrorToSiblings(t *testing.T) {
	srv, ids, _ := readStateTestServer(t, 3)
	srv.bindSink(newFakeSink())
	ctx := context.Background()

	frame := deviceSendFrame{T: "device_send", CID: "cid-0000000000000001", Payload: json.RawMessage(`{"t":"send","text":"hello from A"}`)}
	if err := srv.handleDeviceSend(ctx, ids[0], frame); err != nil {
		t.Fatalf("device_send: %v", err)
	}

	// The sent echo must be in every device's mailbox with origin device=A.
	for i, dev := range ids {
		mb := srv.mailbox.disk.Devices[dev]
		if len(mb.Items) != 1 {
			t.Fatalf("device %d mailbox has %d items, want 1 (the sent mirror)", i, len(mb.Items))
		}
		typ, device, text, cid := sentPayload(t, mb.Items[0])
		if typ != "sent" {
			t.Fatalf("device %d item type = %q, want sent", i, typ)
		}
		if device != ids[0] {
			t.Fatalf("device %d sent.device = %q, want origin %q", i, device, ids[0])
		}
		if text != "hello from A" {
			t.Fatalf("device %d sent.text = %q", i, text)
		}
		if cid != "cid-0000000000000001" {
			t.Fatalf("device %d sent.cid = %q", i, cid)
		}
	}
}

// TestOwnSendMirrorOfflineCatchUp proves an offline sibling — a device provisioned
// AFTER the send (mailbox not yet resident, the wiped/new-device case) — still
// receives the own-send echo, because provision seeds the durable outbox journal
// which includes `sent` frames (durableContent). This is the offline catch-up
// path for the mirror.
func TestOwnSendMirrorOfflineCatchUp(t *testing.T) {
	srv, ids, _ := readStateTestServer(t, 1)
	srv.bindSink(newFakeSink())
	ctx := context.Background()

	frame := deviceSendFrame{T: "device_send", CID: "cid-0000000000000002", Payload: json.RawMessage(`{"t":"send","text":"offline mirror"}`)}
	if err := srv.handleDeviceSend(ctx, ids[0], frame); err != nil {
		t.Fatalf("device_send: %v", err)
	}

	// A brand-new sibling links and provisions AFTER the send. Its mailbox is
	// seeded from the outbox journal, which must include the sent echo.
	link, err := srv.store.MintLink("ws://127.0.0.1:8787", "pi")
	if err != nil {
		t.Fatal(err)
	}
	const late = "dev-f00000000000"
	if res, linked, err := srv.store.VerifyAndLink(link.Room, late, link.Secret); err != nil || !linked || res != VerifyActive {
		t.Fatalf("late link: res=%v linked=%v err=%v", res, linked, err)
	}
	if err := srv.provisionMailbox(late); err != nil {
		t.Fatalf("provision late: %v", err)
	}

	mb := srv.mailbox.disk.Devices[late]
	if len(mb.Items) != 1 {
		t.Fatalf("late device seeded %d items, want 1 (the sent mirror)", len(mb.Items))
	}
	typ, device, text, _ := sentPayload(t, mb.Items[0])
	if typ != "sent" || device != ids[0] || text != "offline mirror" {
		t.Fatalf("late device seed = (%q,%q,%q), want (sent,%q,offline mirror)", typ, device, text, ids[0])
	}
}

// TestUnifiedChatIDStamped proves the env-gated chat_id unification: default-on
// stamps chat_id="app" with user/user_id = origin device; gated off restores the
// legacy per-device chat_id.
func TestUnifiedChatIDStamped(t *testing.T) {
	srv, ids, _ := readStateTestServer(t, 1)
	sink := newFakeSink()
	srv.bindSink(sink)
	ctx := context.Background()

	frame := deviceSendFrame{T: "device_send", CID: "cid-0000000000000003", Payload: json.RawMessage(`{"t":"send","text":"hi"}`)}
	if err := srv.handleDeviceSend(ctx, ids[0], frame); err != nil {
		t.Fatalf("device_send: %v", err)
	}
	cap := <-sink.ch
	if cap.meta["chat_id"] != "app" {
		t.Fatalf("unified chat_id = %q, want app", cap.meta["chat_id"])
	}
	if cap.meta["user"] != ids[0] || cap.meta["user_id"] != ids[0] {
		t.Fatalf("provenance lost: user=%q user_id=%q, want %q", cap.meta["user"], cap.meta["user_id"], ids[0])
	}

	// Gated off: chat_id reverts to the origin device id.
	srv.unifiedChat = false
	frame2 := deviceSendFrame{T: "device_send", CID: "cid-0000000000000004", Payload: json.RawMessage(`{"t":"send","text":"hi2"}`)}
	if err := srv.handleDeviceSend(ctx, ids[0], frame2); err != nil {
		t.Fatalf("device_send 2: %v", err)
	}
	cap2 := <-sink.ch
	if cap2.meta["chat_id"] != ids[0] {
		t.Fatalf("legacy chat_id = %q, want %q", cap2.meta["chat_id"], ids[0])
	}
}
