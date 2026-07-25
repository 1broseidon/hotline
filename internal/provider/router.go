package provider

import (
	"context"
	"fmt"
	"strings"
	"sync"

	"github.com/1broseidon/hotline/internal/mcpchan"
)

// Router multiplexes one or more Providers behind the single MCP tool surface.
//
// Outbound: it implements mcpchan.ToolSet and routes each call by the optional
// "source" argument. With exactly one provider configured, source may be
// omitted and defaults to it — the single-provider setup is byte-compatible
// with the pre-router behavior. With several, source is required (and the tool
// schemas say so; see mcpchan.NewServer).
//
// Inbound: Start runs every provider concurrently, wrapping the shared sink so
// each provider's events are tagged with meta["source"] = its name — the fan-in
// side of the same routing key. All providers share the one sink (the single
// claude/channel stream).
type Router struct {
	order      []string // every provider, inbound Start order
	selectable []string // operator-selectable sources (hidden ones excluded)
	byName     map[string]Provider
}

// hideableProvider is the optional marker a provider implements to stay OUT of the
// operator-facing source set: it is still Start()ed and still refused/routed by
// name, but it never inflates the tool schemas' required "source" enum nor counts
// toward the "multiple channels" default. The fleet channel uses it so an app-only
// box does not suddenly require a source arg on reply/react/… (fleet is reachable
// only via fleet_send).
type hideableProvider interface{ HiddenSource() bool }

// NewRouter builds a Router over the given providers. Names must be unique.
func NewRouter(providers ...Provider) (*Router, error) {
	if len(providers) == 0 {
		return nil, fmt.Errorf("no providers configured")
	}
	r := &Router{byName: make(map[string]Provider, len(providers))}
	for _, p := range providers {
		name := p.Name()
		if name == "" {
			return nil, fmt.Errorf("provider with empty name")
		}
		if _, dup := r.byName[name]; dup {
			return nil, fmt.Errorf("duplicate provider name %q", name)
		}
		r.byName[name] = p
		r.order = append(r.order, name)
		if h, ok := p.(hideableProvider); !ok || !h.HiddenSource() {
			r.selectable = append(r.selectable, name)
		}
	}
	if len(r.selectable) == 0 {
		return nil, fmt.Errorf("no operator-selectable provider configured")
	}
	return r, nil
}

// Sources returns the operator-selectable provider names, in configuration order
// (hidden sources like fleet excluded).
func (r *Router) Sources() []string { return append([]string(nil), r.selectable...) }

// CardSources returns the operator-selectable provider names that can render job
// cards, in configuration order. It is Sources() filtered by the JobCards
// capability: the source set for anything that only makes sense alongside a
// visible card. Empty when no attached channel can show one — callers must then
// stay silent rather than fall back to a channel that cannot render it.
func (r *Router) CardSources() []string {
	var out []string
	for _, name := range r.selectable {
		if r.byName[name].Capabilities().JobCards {
			out = append(out, name)
		}
	}
	return out
}

// PermissionRelay reports whether any provider can relay permission prompts.
func (r *Router) PermissionRelay() bool {
	for _, p := range r.byName {
		if p.Capabilities().PermissionRelay {
			return true
		}
	}
	return false
}

// OnPermissionRequest fans a permission prompt out to every provider that can
// relay it. Any of their authenticated operators may answer; verdict claiming
// stays each provider's job.
func (r *Router) OnPermissionRequest(ctx context.Context, p mcpchan.PermissionRequestParams) {
	for _, name := range r.order {
		if prov := r.byName[name]; prov.Capabilities().PermissionRelay {
			prov.OnPermissionRequest(ctx, p)
		}
	}
}

// Start runs every provider until ctx is cancelled or one gives up. Each
// provider gets the shared sink wrapped to tag its source. The first provider
// error cancels the rest and is returned (the lifecycle treats it as a
// shutdown reason); a clean ctx-driven stop returns nil. Providers that return
// nil early (nothing to poll) don't end the run — Start keeps the process
// alive until ctx is done, preserving handshake-only mode.
func (r *Router) Start(ctx context.Context, sink InboundSink) error {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	errCh := make(chan error, len(r.order))
	var wg sync.WaitGroup
	for _, name := range r.order {
		p := r.byName[name]
		wg.Add(1)
		go func() {
			defer wg.Done()
			if err := p.Start(ctx, &taggedSink{source: p.Name(), base: sink}); err != nil {
				errCh <- fmt.Errorf("provider %s: %w", p.Name(), err)
			}
		}()
	}

	var err error
	select {
	case err = <-errCh:
		cancel() // one gave up — stop the others
	case <-ctx.Done():
	}
	wg.Wait()
	return err
}

// taggedSink stamps meta["source"] with the provider's name on the channel
// path and passes verdicts through untouched.
type taggedSink struct {
	source string
	base   InboundSink
}

func (s *taggedSink) SendChannel(ctx context.Context, content string, meta map[string]string) error {
	m := make(map[string]string, len(meta)+1)
	for k, v := range meta {
		m[k] = v
	}
	if m["source"] == "" {
		m["source"] = s.source
	}
	return s.base.SendChannel(ctx, content, m)
}

func (s *taggedSink) SendVerdict(ctx context.Context, requestID, behavior string) error {
	return s.base.SendVerdict(ctx, requestID, behavior)
}

// pick resolves the provider an outbound tool call targets. An empty source is
// allowed only when exactly one provider is configured.
func (r *Router) pick(tool, source string) (Provider, string) {
	if source == "" {
		if len(r.selectable) == 1 {
			return r.byName[r.selectable[0]], ""
		}
		return nil, fmt.Sprintf("%s failed: multiple channels connected — pass source (one of: %s)",
			tool, strings.Join(r.selectable, ", "))
	}
	p, ok := r.byName[source]
	if !ok {
		return nil, fmt.Sprintf("%s failed: unknown source %q (configured: %s)",
			tool, source, strings.Join(r.selectable, ", "))
	}
	return p, ""
}

// fleetRefusal is the exact F11 message every operator tool returns when its
// target is a fleet chat (source="fleet" or a chat_id in the fleet: namespace).
const fleetRefusal = "use fleet_send for fleet peers"

// refuseFleet reports whether an operator tool call targets a fleet chat. The
// router is the single earliest common point every routed operator tool passes
// through, so the guard lives here (reply/react/edit/download/publish/job) rather
// than in each provider.
func refuseFleet(source, chatID string) bool {
	return source == "fleet" || strings.HasPrefix(chatID, "fleet:")
}

// Reply implements mcpchan.ToolSet.
func (r *Router) Reply(ctx context.Context, in mcpchan.ReplyInput) (string, bool) {
	if refuseFleet(in.Source, in.ChatID) {
		return fleetRefusal, true
	}
	p, errMsg := r.pick("reply", in.Source)
	if p == nil {
		return errMsg, true
	}
	return p.Reply(ctx, in)
}

// React implements mcpchan.ToolSet.
func (r *Router) React(ctx context.Context, in mcpchan.ReactInput) (string, bool) {
	if refuseFleet(in.Source, in.ChatID) {
		return fleetRefusal, true
	}
	p, errMsg := r.pick("react", in.Source)
	if p == nil {
		return errMsg, true
	}
	return p.React(ctx, in)
}

// EditMessage implements mcpchan.ToolSet.
func (r *Router) EditMessage(ctx context.Context, in mcpchan.EditInput) (string, bool) {
	if refuseFleet(in.Source, in.ChatID) {
		return fleetRefusal, true
	}
	p, errMsg := r.pick("edit_message", in.Source)
	if p == nil {
		return errMsg, true
	}
	return p.EditMessage(ctx, in)
}

// DownloadAttachment implements mcpchan.ToolSet.
func (r *Router) DownloadAttachment(ctx context.Context, in mcpchan.DownloadInput) (string, bool) {
	if refuseFleet(in.Source, "") {
		return fleetRefusal, true
	}
	p, errMsg := r.pick("download_attachment", in.Source)
	if p == nil {
		return errMsg, true
	}
	return p.DownloadAttachment(ctx, in)
}

// PublishArtifact implements mcpchan.ArtifactPublisher without widening
// ToolSet. Only a provider that implements the optional interface gets first
// refusal; every other target falls back to the existing exposure backend.
func (r *Router) PublishArtifact(ctx context.Context, in mcpchan.PublishInput) (string, bool, bool) {
	if refuseFleet(in.Source, in.ChatID) {
		return fleetRefusal, true, true
	}
	p, errMsg := r.pick("publish", in.Source)
	if p == nil {
		return errMsg, true, true
	}
	publisher, ok := p.(mcpchan.ArtifactPublisher)
	if !ok {
		return "", false, false
	}
	return publisher.PublishArtifact(ctx, in)
}

// Job implements mcpchan.JobRunner without widening ToolSet. Only a provider
// that owns a job registry (the app channel) gets first refusal; every other
// target reports handled=false so the MCP layer can say the tool is
// unavailable there.
func (r *Router) Job(ctx context.Context, in mcpchan.JobInput) (string, bool, bool) {
	if refuseFleet(in.Source, in.ChatID) {
		return fleetRefusal, true, true
	}
	p, errMsg := r.pick("job", in.Source)
	if p == nil {
		return errMsg, true, true
	}
	runner, ok := p.(mcpchan.JobRunner)
	if !ok {
		return "", false, false
	}
	return runner.Job(ctx, in)
}
