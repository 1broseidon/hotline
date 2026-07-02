package signal

import (
	"context"
	"encoding/base64"
	"testing"
)

func TestSendDMRequestShape(t *testing.T) {
	d := newFakeDaemon(t)
	c := NewClient(d.srv.URL, testAccount)

	ts, err := c.Send(context.Background(), "+15550009999", "hello", nil, 0)
	if err != nil {
		t.Fatal(err)
	}
	if ts == 0 {
		t.Fatal("no timestamp returned")
	}
	calls := d.callsFor("send")
	if len(calls) != 1 {
		t.Fatalf("calls %v", calls)
	}
	p := calls[0].Params
	rec, ok := p["recipient"].([]any)
	if !ok || len(rec) != 1 || rec[0] != "+15550009999" {
		t.Fatalf("recipient %v", p["recipient"])
	}
	if p["message"] != "hello" {
		t.Fatalf("message %v", p["message"])
	}
	if _, has := p["groupId"]; has {
		t.Fatal("DM send must not carry groupId")
	}
	if _, has := p["editTimestamp"]; has {
		t.Fatal("plain send must not carry editTimestamp")
	}
	if _, has := p["account"]; has {
		t.Fatal("single-account daemon requests must not carry account")
	}
}

func TestSendGroupRequestShape(t *testing.T) {
	d := newFakeDaemon(t)
	c := NewClient(d.srv.URL, testAccount)

	if _, err := c.Send(context.Background(), "group:AbC123==", "yo", nil, 0); err != nil {
		t.Fatal(err)
	}
	p := d.callsFor("send")[0].Params
	if p["groupId"] != "AbC123==" {
		t.Fatalf("groupId %v", p["groupId"])
	}
	if _, has := p["recipient"]; has {
		t.Fatal("group send must not carry recipient")
	}
}

func TestSendEditCarriesEditTimestamp(t *testing.T) {
	d := newFakeDaemon(t)
	c := NewClient(d.srv.URL, testAccount)

	if _, err := c.Send(context.Background(), "+15550009999", "fixed", nil, 1699999999999); err != nil {
		t.Fatal(err)
	}
	p := d.callsFor("send")[0].Params
	if p["editTimestamp"] != float64(1699999999999) {
		t.Fatalf("editTimestamp %v", p["editTimestamp"])
	}
}

func TestSendReactionRequestShape(t *testing.T) {
	d := newFakeDaemon(t)
	c := NewClient(d.srv.URL, testAccount)

	if err := c.SendReaction(context.Background(), "+15550009999", "👍", "+15550009999", 1700000001234); err != nil {
		t.Fatal(err)
	}
	p := d.callsFor("sendReaction")[0].Params
	if p["emoji"] != "👍" || p["targetAuthor"] != "+15550009999" || p["targetTimestamp"] != float64(1700000001234) {
		t.Fatalf("params %v", p)
	}
}

func TestSendTypingRequestShape(t *testing.T) {
	d := newFakeDaemon(t)
	c := NewClient(d.srv.URL, testAccount)

	if err := c.SendTyping(context.Background(), "group:g1"); err != nil {
		t.Fatal(err)
	}
	p := d.callsFor("sendTyping")[0].Params
	if p["groupId"] != "g1" {
		t.Fatalf("params %v", p)
	}
}

func TestGetAttachmentDecodesBase64(t *testing.T) {
	d := newFakeDaemon(t)
	d.Results["getAttachment"] = base64.StdEncoding.EncodeToString([]byte("PNGDATA"))
	c := NewClient(d.srv.URL, testAccount)

	data, err := c.GetAttachment(context.Background(), "+15550009999", "att-1")
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != "PNGDATA" {
		t.Fatalf("data %q", data)
	}
	p := d.callsFor("getAttachment")[0].Params
	if p["id"] != "att-1" {
		t.Fatalf("params %v", p)
	}
}

func TestCallSurfacesRPCError(t *testing.T) {
	d := newFakeDaemon(t)
	d.Fail["send"] = "Invalid group id"
	c := NewClient(d.srv.URL, testAccount)

	if _, err := c.Send(context.Background(), "group:???", "x", nil, 0); err == nil {
		t.Fatal("rpc error not surfaced")
	}
}

func TestParseMessageID(t *testing.T) {
	ts, author, ok := parseMessageID("1700000000123:+15550009999")
	if !ok || ts != 1700000000123 || author != "+15550009999" {
		t.Fatalf("got %d %q %v", ts, author, ok)
	}
	ts, author, ok = parseMessageID("1700000000123")
	if !ok || ts != 1700000000123 || author != "" {
		t.Fatalf("bare: %d %q %v", ts, author, ok)
	}
	if _, _, ok := parseMessageID("nonsense"); ok {
		t.Fatal("nonsense accepted")
	}
}
