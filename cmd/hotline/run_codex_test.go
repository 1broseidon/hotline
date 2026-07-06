package main

import (
	"context"
	"errors"
	"testing"

	"github.com/1broseidon/hotline/internal/harness"
	"github.com/1broseidon/hotline/internal/provider"
	"github.com/1broseidon/hotline/internal/provider/stubprovider"
)

type errLink struct {
	err   error
	perms chan harness.PermissionRequest
}

func (e *errLink) Start(context.Context) error                        { return e.err }
func (e *errLink) PushInbound(context.Context, harness.Inbound) error { return nil }
func (e *errLink) Permissions() <-chan harness.PermissionRequest      { return e.perms }
func (e *errLink) AnswerPermission(context.Context, string, bool) error {
	return nil
}

func TestRunCodexLoopReturnsLinkError(t *testing.T) {
	want := errors.New("boom")
	link := &errLink{err: want, perms: make(chan harness.PermissionRequest)}
	router, err := provider.NewRouter(&stubprovider.Stub{ProviderName: "stub"})
	if err != nil {
		t.Fatal(err)
	}

	got := runCodexLoop(context.Background(), router, link, false, &codexSink{link: link})
	if !errors.Is(got, want) {
		t.Fatalf("runCodexLoop error = %v, want %v", got, want)
	}
}
