package provider_test

import (
	"context"
	"testing"

	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/provider"
	"github.com/1broseidon/hotline/internal/provider/stubprovider"
)

// hiddenStub is a provider that opts out of the operator-selectable source set
// (the fleet channel's posture).
type hiddenStub struct{ *stubprovider.Stub }

func (hiddenStub) HiddenSource() bool { return true }

const fleetRefusal = "use fleet_send for fleet peers"

// TestRouterRefusesFleetChats proves F11: every operator tool refuses a fleet
// target (source="fleet" or a fleet:* chat_id) with the exact message, and the
// underlying provider is never invoked.
func TestRouterRefusesFleetChats(t *testing.T) {
	app := &stubprovider.Stub{ProviderName: "app"}
	r, err := provider.NewRouter(app)
	if err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()

	check := func(name, msg string, isErr bool) {
		if !isErr || msg != fleetRefusal {
			t.Fatalf("%s: got (%q, isErr=%t), want (%q, true)", name, msg, isErr, fleetRefusal)
		}
	}

	// By fleet:* chat_id.
	m, e := r.Reply(ctx, mcpchan.ReplyInput{ChatID: "fleet:abcd1234", Text: "x"})
	check("reply chat_id", m, e)
	m, e = r.React(ctx, mcpchan.ReactInput{ChatID: "fleet:abcd1234", MessageID: "a-1", Emoji: "👍"})
	check("react chat_id", m, e)
	m, e = r.EditMessage(ctx, mcpchan.EditInput{ChatID: "fleet:abcd1234", MessageID: "a-1", Text: "x"})
	check("edit chat_id", m, e)
	pm, pe, _ := r.PublishArtifact(ctx, mcpchan.PublishInput{ChatID: "fleet:abcd1234", Path: "/tmp/x.html"})
	check("publish chat_id", pm, pe)
	jm, je, _ := r.Job(ctx, mcpchan.JobInput{ChatID: "fleet:abcd1234", Action: "start", Title: "x"})
	check("job chat_id", jm, je)

	// By source="fleet".
	m, e = r.Reply(ctx, mcpchan.ReplyInput{Source: "fleet", ChatID: "app", Text: "x"})
	check("reply source", m, e)
	m, e = r.DownloadAttachment(ctx, mcpchan.DownloadInput{Source: "fleet", FileID: "f"})
	check("download source", m, e)

	// The operator provider never saw any of it.
	if len(app.Replies)+len(app.Reacts)+len(app.Edits)+len(app.Downloads) != 0 {
		t.Fatalf("operator provider was invoked on a refused fleet call")
	}
}

// TestRouterHiddenSourceExcludedFromSources proves the fleet channel is registered
// (Start-able) but excluded from the operator source set, so an app-only box keeps
// a single selectable source (no forced source arg on reply/…).
func TestRouterHiddenSourceExcludedFromSources(t *testing.T) {
	app := &stubprovider.Stub{ProviderName: "app"}
	fleet := hiddenStub{&stubprovider.Stub{ProviderName: "fleet"}}
	r, err := provider.NewRouter(app, fleet)
	if err != nil {
		t.Fatal(err)
	}
	srcs := r.Sources()
	if len(srcs) != 1 || srcs[0] != "app" {
		t.Fatalf("Sources() = %v, want [app] (fleet hidden)", srcs)
	}
	// A source-less reply still routes to the single selectable provider.
	if _, isErr := r.Reply(context.Background(), mcpchan.ReplyInput{ChatID: "1", Text: "hi"}); isErr {
		t.Fatalf("source-less reply failed with a hidden second source present")
	}
	if len(app.Replies) != 1 {
		t.Fatalf("reply did not route to app; replies=%d", len(app.Replies))
	}
}
