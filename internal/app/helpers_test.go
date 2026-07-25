package app

import (
	"context"
	"encoding/json"
	"path/filepath"
	"testing"

	"github.com/1broseidon/hotline/internal/access"
	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/transcript"
)

// testConfig builds a minimal app Config rooted at a temp dir.
func testConfig(t *testing.T) *config.Config {
	t.Helper()
	dir := t.TempDir()
	return &config.Config{
		StateDir:       dir,
		StateRoot:      dir,
		AccessFile:     filepath.Join(dir, "access.json"),
		InboxDir:       filepath.Join(dir, "inbox"),
		TranscriptFile: filepath.Join(dir, "transcript.jsonl"),
		AppBind:        "127.0.0.1:0",
	}
}

// writeAccess persists an access document (used to seed allowFrom / bubbleMode).
func writeAccess(t *testing.T, cfg *config.Config, a *access.Access) {
	t.Helper()
	if err := access.Save(a, cfg.AccessFile); err != nil {
		t.Fatalf("save access: %v", err)
	}
}

// allowlisted returns a default access doc with the given device ids allowed.
func allowlisted(ids ...string) *access.Access {
	a := access.Defaults()
	a.AllowFrom = append(a.AllowFrom, ids...)
	return a
}

// fakeSink captures SendChannel calls for assertions.
type fakeSink struct {
	ch chan capture
}

type capture struct {
	content string
	meta    map[string]string
}

func newFakeSink() *fakeSink { return &fakeSink{ch: make(chan capture, 8)} }

func (f *fakeSink) SendChannel(_ context.Context, content string, meta map[string]string) error {
	f.ch <- capture{content: content, meta: meta}
	return nil
}

func (f *fakeSink) SendVerdict(_ context.Context, _, _ string) error { return nil }

// newTools builds a Tools + Server pair over cfg with a discarded transcript.
func newTools(cfg *config.Config) (*Server, *Tools) {
	log := transcript.New(cfg.TranscriptFile)
	srv := NewServer(cfg, log)
	return srv, NewTools(srv, cfg, log)
}

// decodeFrame unmarshals a stored frame into a generic map.
func decodeFrame(t *testing.T, data []byte) map[string]any {
	t.Helper()
	var m map[string]any
	if err := json.Unmarshal(data, &m); err != nil {
		t.Fatalf("decode frame: %v", err)
	}
	return m
}
