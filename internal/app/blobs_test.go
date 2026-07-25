package app

import (
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestBlobOutboundChunksAt256KiBAndResends(t *testing.T) {
	dir := t.TempDir()
	data := make([]byte, blobChunkBytes+17)
	for i := range data {
		data[i] = byte(i)
	}
	path := filepath.Join(dir, "artifact.bin")
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatal(err)
	}
	b := newBlobRegistry(dir)
	rec, err := b.register(path, "application/octet-stream")
	if err != nil {
		t.Fatal(err)
	}
	frames, err := b.frames(rec.Xfer)
	if err != nil {
		t.Fatal(err)
	}
	if len(frames) != 4 {
		t.Fatalf("frames = %d, want begin + 2 chunks + end", len(frames))
	}
	for _, raw := range frames[1:3] {
		var f struct {
			Data string `json:"data"`
		}
		if err := json.Unmarshal(raw, &f); err != nil {
			t.Fatal(err)
		}
		chunk, err := base64.StdEncoding.DecodeString(f.Data)
		if err != nil || len(chunk) > blobChunkBytes {
			t.Fatalf("chunk size=%d err=%v", len(chunk), err)
		}
	}
	b2 := newBlobRegistry(dir)
	if again, err := b2.frames(rec.Xfer); err != nil || len(again) != len(frames) {
		t.Fatalf("restart resend frames=%d err=%v", len(again), err)
	}
}

func TestBlobInboundReassemblySizeAndExpiry(t *testing.T) {
	b := newBlobRegistry(t.TempDir())
	if err := b.begin("x-inbound", "image/jpeg", 5, 2); err != nil {
		t.Fatal(err)
	}
	if err := b.chunk("x-inbound", 0, base64.StdEncoding.EncodeToString([]byte("hel"))); err != nil {
		t.Fatal(err)
	}
	if err := b.chunk("x-inbound", 1, base64.StdEncoding.EncodeToString([]byte("lo"))); err != nil {
		t.Fatal(err)
	}
	rec, err := b.end("x-inbound")
	if err != nil {
		t.Fatal(err)
	}
	if data, _ := os.ReadFile(rec.Path); string(data) != "hello" {
		t.Fatalf("reassembled = %q", data)
	}
	// Completed client uploads become durable outbound transfers too: a sent.file
	// echo can replay the bytes to sibling/reconnecting clients after restart.
	b2 := newBlobRegistry(b.stateDir)
	if restored, ok := b2.resolve("x-inbound"); !ok || restored.Path != rec.Path {
		t.Fatalf("restored completed upload = %+v ok=%v", restored, ok)
	}
	if frames, err := b2.frames("x-inbound"); err != nil || len(frames) != 3 {
		t.Fatalf("restored upload frames=%d err=%v", len(frames), err)
	}

	now := time.Now()
	b.clock = func() time.Time { return now }
	if err := b.begin("x-expired", "image/png", 1, 1); err != nil {
		t.Fatal(err)
	}
	now = now.Add(blobExpiry)
	if err := b.chunk("x-expired", 0, base64.StdEncoding.EncodeToString([]byte("x"))); err == nil {
		t.Fatal("expired transfer accepted a chunk")
	}
}
