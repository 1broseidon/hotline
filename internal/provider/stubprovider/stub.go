// Package stubprovider is a minimal in-memory provider used by tests: it
// records outbound tool calls, replays scripted inbound events on Start, and
// exercises the capability-degradation path (buttons rendered as numbered
// text options when Capabilities.Buttons is false).
package stubprovider

import (
	"context"
	"sync"

	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/provider"
)

// Inbound is one scripted inbound event delivered to the sink on Start.
type Inbound struct {
	Content string
	Meta    map[string]string
}

// Stub implements provider.Provider in memory.
type Stub struct {
	ProviderName string
	Caps         provider.Capabilities
	// InboundEvents are delivered to the sink, in order, when Start runs.
	InboundEvents []Inbound

	mu sync.Mutex
	// Sent records the rendered text of each Reply, post-degradation.
	Sent []string
	// Replies, Reacts, Edits, Downloads record the raw inputs each tool saw.
	Replies   []mcpchan.ReplyInput
	Reacts    []mcpchan.ReactInput
	Edits     []mcpchan.EditInput
	Downloads []mcpchan.DownloadInput
	// PermRequests records relayed permission prompts.
	PermRequests []mcpchan.PermissionRequestParams
}

// Name implements provider.Provider.
func (s *Stub) Name() string { return s.ProviderName }

// Capabilities implements provider.Provider.
func (s *Stub) Capabilities() provider.Capabilities { return s.Caps }

// Start implements provider.Provider: it delivers the scripted inbound events
// and returns (tests drive lifetimes via the router, which keeps running until
// its ctx is cancelled).
func (s *Stub) Start(ctx context.Context, sink provider.InboundSink) error {
	for _, ev := range s.InboundEvents {
		if err := sink.SendChannel(ctx, ev.Content, ev.Meta); err != nil {
			return err
		}
	}
	return nil
}

// OnPermissionRequest implements provider.Provider.
func (s *Stub) OnPermissionRequest(_ context.Context, p mcpchan.PermissionRequestParams) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.PermRequests = append(s.PermRequests, p)
}

// Reply implements mcpchan.ToolSet, degrading buttons to numbered text options
// when the stub's capabilities lack native buttons — the same contract a real
// button-less transport adapter must honor.
func (s *Stub) Reply(_ context.Context, in mcpchan.ReplyInput) (string, bool) {
	text := in.Text
	if len(in.Bubbles) > 0 {
		text = in.Bubbles[len(in.Bubbles)-1]
	}
	if len(in.Buttons) > 0 && !s.Caps.Buttons {
		text += "\n" + provider.ButtonsToNumberedText(in.Buttons)
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	s.Replies = append(s.Replies, in)
	s.Sent = append(s.Sent, text)
	return "sent", false
}

// React implements mcpchan.ToolSet.
func (s *Stub) React(_ context.Context, in mcpchan.ReactInput) (string, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.Reacts = append(s.Reacts, in)
	return "reacted", false
}

// EditMessage implements mcpchan.ToolSet.
func (s *Stub) EditMessage(_ context.Context, in mcpchan.EditInput) (string, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.Edits = append(s.Edits, in)
	return "edited", false
}

// DownloadAttachment implements mcpchan.ToolSet.
func (s *Stub) DownloadAttachment(_ context.Context, in mcpchan.DownloadInput) (string, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.Downloads = append(s.Downloads, in)
	return "/tmp/stub-download", false
}
