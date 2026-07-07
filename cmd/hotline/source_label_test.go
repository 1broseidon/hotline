package main

import (
	"context"
	"testing"
)

// recordSink captures the (content, meta) a wrapped sink forwards.
type recordSink struct {
	content string
	meta    map[string]string
}

func (r *recordSink) SendChannel(_ context.Context, content string, meta map[string]string) error {
	r.content, r.meta = content, meta
	return nil
}
func (r *recordSink) SendVerdict(_ context.Context, _, _ string) error { return nil }

func TestSourceLabelSinkDecoratesUser(t *testing.T) {
	rec := &recordSink{}
	s := &sourceLabelSink{next: rec}

	orig := map[string]string{"user": "George", "source": "signal", "chat_id": "c1"}
	if err := s.SendChannel(context.Background(), "hi", orig); err != nil {
		t.Fatal(err)
	}
	if got := rec.meta["user"]; got != "George · signal" {
		t.Errorf("user = %q, want %q", got, "George · signal")
	}
	if rec.meta["source"] != "signal" || rec.meta["chat_id"] != "c1" {
		t.Errorf("other meta keys must pass through, got %v", rec.meta)
	}
	// The caller's map must not be mutated: providers may reuse it.
	if orig["user"] != "George" {
		t.Errorf("original meta mutated: user = %q", orig["user"])
	}
}

func TestSourceLabelSinkLeavesUserlessMetaAlone(t *testing.T) {
	rec := &recordSink{}
	s := &sourceLabelSink{next: rec}

	// A schedule fire has source but no user; a hypothetical userless event
	// must pass through untouched (same map, no decoration).
	meta := map[string]string{"source": "signal", "kind": "schedule"}
	if err := s.SendChannel(context.Background(), "fire", meta); err != nil {
		t.Fatal(err)
	}
	if _, ok := rec.meta["user"]; ok {
		t.Errorf("user key invented: %v", rec.meta)
	}
	// And user without source is also left alone.
	meta2 := map[string]string{"user": "George"}
	_ = s.SendChannel(context.Background(), "x", meta2)
	if rec.meta["user"] != "George" {
		t.Errorf("user without source must be undecorated, got %q", rec.meta["user"])
	}
}
