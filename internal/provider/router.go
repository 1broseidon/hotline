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
	order  []string
	byName map[string]Provider
}

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
	}
	return r, nil
}

// Sources returns the configured provider names, in configuration order.
func (r *Router) Sources() []string { return append([]string(nil), r.order...) }

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
		if len(r.order) == 1 {
			return r.byName[r.order[0]], ""
		}
		return nil, fmt.Sprintf("%s failed: multiple channels connected — pass source (one of: %s)",
			tool, strings.Join(r.order, ", "))
	}
	p, ok := r.byName[source]
	if !ok {
		return nil, fmt.Sprintf("%s failed: unknown source %q (configured: %s)",
			tool, source, strings.Join(r.order, ", "))
	}
	return p, ""
}

// Reply implements mcpchan.ToolSet.
func (r *Router) Reply(ctx context.Context, in mcpchan.ReplyInput) (string, bool) {
	p, errMsg := r.pick("reply", in.Source)
	if p == nil {
		return errMsg, true
	}
	return p.Reply(ctx, in)
}

// React implements mcpchan.ToolSet.
func (r *Router) React(ctx context.Context, in mcpchan.ReactInput) (string, bool) {
	p, errMsg := r.pick("react", in.Source)
	if p == nil {
		return errMsg, true
	}
	return p.React(ctx, in)
}

// EditMessage implements mcpchan.ToolSet.
func (r *Router) EditMessage(ctx context.Context, in mcpchan.EditInput) (string, bool) {
	p, errMsg := r.pick("edit_message", in.Source)
	if p == nil {
		return errMsg, true
	}
	return p.EditMessage(ctx, in)
}

// DownloadAttachment implements mcpchan.ToolSet.
func (r *Router) DownloadAttachment(ctx context.Context, in mcpchan.DownloadInput) (string, bool) {
	p, errMsg := r.pick("download_attachment", in.Source)
	if p == nil {
		return errMsg, true
	}
	return p.DownloadAttachment(ctx, in)
}
